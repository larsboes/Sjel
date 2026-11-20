//! Document proposal reviews and local rule and model generation.

use std::collections::BTreeMap;

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight};
use sjel_gpui_core::shell::{
    self, DocumentDetail, ProposalEvidenceSource, ProposalOutcome, ProposalReview, ProposalSource,
    ShellError,
};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

use super::Documents;

fn proposal_source_label(source: &ProposalSource) -> String {
    match source {
        ProposalSource::Rules { identity, version } => format!("Rules · {identity} · {version}"),
        ProposalSource::Model {
            role,
            model,
            revision,
        } => format!(
            "Model · {role} · {model}{}",
            revision
                .as_deref()
                .map(|revision| format!(" · {revision}"))
                .unwrap_or_default()
        ),
    }
}

fn proposal_outcome_label(outcome: &ProposalOutcome) -> String {
    match outcome {
        ProposalOutcome::Proposed { category, .. } => category.clone(),
        ProposalOutcome::Abstained { reason } => format!(
            "No category · {}",
            match reason {
                shell::ProposalAbstentionReason::NoMatch => "no rule matched",
                shell::ProposalAbstentionReason::MissingEvidence => "evidence missing",
                shell::ProposalAbstentionReason::ModelUnavailable => "source unavailable",
                shell::ProposalAbstentionReason::CategoryNotConfigured => "category not configured",
                shell::ProposalAbstentionReason::SafetyRefused => "safety refused",
            }
        ),
        ProposalOutcome::Invalid { reason } => format!(
            "Invalid · {}",
            match reason {
                shell::ProposalInvalidReason::MalformedOutput => "malformed output",
                shell::ProposalInvalidReason::EvidenceNotExact => "evidence was not exact",
                shell::ProposalInvalidReason::EvidenceTooLong => "evidence exceeded limits",
                shell::ProposalInvalidReason::InvalidCategory => "category was invalid",
            }
        ),
    }
}

fn proposal_review_label(review: ProposalReview) -> &'static str {
    match review {
        ProposalReview::Accepted => "Agreed",
        ProposalReview::Rejected => "Rejected",
        ProposalReview::Overridden => "Overridden",
    }
}

pub(super) fn proposal_panel(
    view: &Documents,
    detail: &DocumentDetail,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let latest_generation = detail
        .proposals
        .iter()
        .max_by(|left, right| {
            left.generated_at
                .cmp(&right.generated_at)
                .then_with(|| left.generation_id.cmp(&right.generation_id))
        })
        .map(|record| record.generation_id.as_str());
    let latest: Vec<_> = detail
        .proposals
        .iter()
        .filter(|record| Some(record.generation_id.as_str()) == latest_generation)
        .collect();
    let mut categories = BTreeMap::<String, usize>::new();
    for record in &latest {
        if let ProposalOutcome::Proposed { category, .. } = &record.proposal.outcome {
            *categories.entry(category.clone()).or_default() += 1;
        }
    }
    let proposed_count: usize = categories.values().sum();
    let summary = if categories.len() > 1 {
        format!(
            "Sources disagree: {}",
            categories.keys().cloned().collect::<Vec<_>>().join(" · ")
        )
    } else if proposed_count > 1 {
        format!(
            "Sources agree: {}",
            categories.keys().next().cloned().unwrap_or_default()
        )
    } else if proposed_count == 1 {
        format!(
            "One proposal: {}",
            categories.keys().next().cloned().unwrap_or_default()
        )
    } else if latest.is_empty() {
        "No proposal records yet".into()
    } else {
        "No category proposed in this run".into()
    };
    let older_runs = detail
        .proposals
        .iter()
        .filter(|record| Some(record.generation_id.as_str()) != latest_generation)
        .map(|record| record.generation_id.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut visible: Vec<_> = detail
        .proposals
        .iter()
        .filter(|record| {
            view.show_proposal_history || Some(record.generation_id.as_str()) == latest_generation
        })
        .collect();
    visible.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let busy = view.proposal_busy.is_some();
    let mut panel = div()
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded(px(6.))
        .bg(rgba(palette.raised))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Classification proposals"),
                )
                .child(div().flex_1())
                .child(
                    Button::new("document-regenerate-proposals", if view.proposal_busy.as_deref() == Some("regenerate") {
                            "Saving…"
                        } else {
                            "Regenerate rules"
                        })
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.regenerate_proposals(cx))),
                )
                .child(
                    Button::new("document-model-proposal", if view.proposal_busy.as_deref() == Some("model") {
                            "Asking local model…"
                        } else {
                            "Ask local model"
                        })
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.generate_model_proposal(cx))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("A local model can suggest a category and exact text evidence. Review it before filing."),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(if categories.len() > 1 {
                    palette.amber
                } else {
                    palette.muted
                }))
                .child(summary),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Review records an assessment only. Filing remains a separate action."),
        );
    if let Some(error) = view.proposal_error.as_ref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(format!("Proposal action failed: {error}")),
        );
    }
    if let Some(notice) = view.proposal_notice.as_ref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(notice.clone()),
        );
    }
    if older_runs > 0 {
        panel = panel.child(
            Button::new(
                "document-toggle-proposal-history",
                if view.show_proposal_history {
                    "Show latest run only"
                } else {
                    "Show earlier runs"
                },
            )
            .size(ControlSize::Sm)
            .variant(ButtonVariant::Ghost)
            .on_click(cx.listener(|view, _, _, cx| view.toggle_proposal_history(cx))),
        );
    }
    if visible.is_empty() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Generate a rules run to record a source result."),
        );
    }
    panel
        .children(visible.into_iter().map(|record| {
            let source = proposal_source_label(&record.proposal.source);
            let outcome = proposal_outcome_label(&record.proposal.outcome);
            let evidence = match &record.proposal.outcome {
                ProposalOutcome::Proposed { evidence, .. } => Some((
                    match evidence.source {
                        ProposalEvidenceSource::Filename => "Filename evidence",
                        ProposalEvidenceSource::ExtractedText => "Text evidence",
                    },
                    evidence.quote.clone(),
                )),
                _ => None,
            };
            let review_history = if record.review_history.is_empty() {
                "Not reviewed".to_owned()
            } else {
                record
                    .review_history
                    .iter()
                    .map(|event| {
                        format!(
                            "{} · {}",
                            proposal_review_label(event.outcome),
                            event.reviewed_at.get(..10).unwrap_or("date unknown")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" → ")
            };
            let proposal_id = record.id.clone();
            let source_id = record.id.clone();
            let override_id = record.id.clone();
            let latest_review = record
                .proposal
                .review
                .map(proposal_review_label)
                .unwrap_or("Not reviewed");
            let confidence = record
                .proposal
                .confidence
                .map(|confidence| format!("Confidence {confidence:.2}"));
            let generated = record
                .generated_at
                .get(..16)
                .unwrap_or(&record.generated_at);
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(4.))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(source),
                        )
                        .child(div().text_xs().child(outcome))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(latest_review),
                        )
                        .when_some(confidence, |row, confidence| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(confidence),
                            )
                        }),
                )
                .when_some(evidence, |row, (label, quote)| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{label}: “{quote}”")),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Run {generated} · review history: {review_history}"
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new(
                                gpui::SharedString::from(format!(
                                    "document-proposal-agree-{}",
                                    record.id
                                )),
                                "Agree",
                            )
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.review_proposal(
                                        proposal_id.clone(),
                                        ProposalReview::Accepted,
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                gpui::SharedString::from(format!(
                                    "document-proposal-reject-{}",
                                    source_id
                                )),
                                "Reject",
                            )
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.review_proposal(
                                        source_id.clone(),
                                        ProposalReview::Rejected,
                                        cx,
                                    )
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                gpui::SharedString::from(format!(
                                    "document-proposal-override-{}",
                                    override_id
                                )),
                                "Override",
                            )
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(busy)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.review_proposal(
                                        override_id.clone(),
                                        ProposalReview::Overridden,
                                        cx,
                                    )
                                },
                            )),
                        ),
                )
        }))
        .into_any_element()
}

pub(super) fn workflow_panel(
    view: &Documents,
    detail: &DocumentDetail,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let document = &detail.document;
    let is_pdf = document.media_type == "application/pdf";
    let mut panel = div().flex().flex_col().gap_1();

    if is_pdf {
        let analyzing = view.workflow_busy.as_deref() == Some("analyze");
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new(
                        "document-analyze-pages",
                        if analyzing {
                            "Analyzing locally…"
                        } else if document.page_count == 0 {
                            "Analyze pages locally"
                        } else {
                            "Refresh page analysis"
                        },
                    )
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(|view, _, _, cx| view.analyze_selected_document(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} analyzed page(s)", document.page_count)),
                ),
        );

        if super::split_picker::can_split(document)
            && detail.children.is_empty()
            && view.reader.pages > 1
        {
            panel = panel.child(
                div()
                    .debug_selector(|| "document-pick-split-pages-control".into())
                    .child(
                        Button::new("document-pick-split-pages", "Pick split pages")
                            .size(ControlSize::Sm)
                            .disabled(view.workflow_busy.is_some())
                            .on_click(cx.listener(|view, _, _, cx| view.begin_split_picker(cx))),
                    ),
            );
        }

        if !detail.children.is_empty() {
            panel = panel.child(
                div()
                    .mt(px(3.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Child records · independent PDFs linked to the source"),
            );
            for child in &detail.children {
                let id = child.id.clone();
                let label = child.page_range.map_or_else(
                    || child.filename.clone(),
                    |range| format!("Pages {}-{} · {}", range.first, range.last, child.filename),
                );
                panel = panel.child(
                    Button::new(
                        gpui::SharedString::from(format!("document-child-{}", &child.id[..12])),
                        label,
                    )
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(move |view, _, _, cx| view.open(id.clone(), cx))),
                );
            }
        }

        if !detail.pages.is_empty() {
            panel = panel.child(
                div()
                    .mt(px(3.))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("Page evidence"),
            );
            for page in &detail.pages {
                let preview = page
                    .text
                    .split_whitespace()
                    .take(28)
                    .collect::<Vec<_>>()
                    .join(" ");
                let provenance = page.failure.as_ref().map_or_else(
                    || format!("{} · {} chars", page.engine, page.text.chars().count()),
                    |failure| format!("{} · extraction issue: {failure}", page.engine),
                );
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .rounded(px(4.))
                        .px_2()
                        .py(px(2.))
                        .bg(rgba(palette.raised))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("Page {} · {provenance}", page.page_number)),
                        )
                        .child(div().text_xs().child(preview)),
                );
            }
        }
    }

    if let Some(error) = &view.workflow_error {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(error.clone()),
        );
    }
    if is_pdf && document.parent_id.is_some() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "Child of {} · pages {}-{}",
                    document.parent_id.as_deref().unwrap_or("unknown"),
                    document.page_range.map_or(0, |range| range.first),
                    document.page_range.map_or(0, |range| range.last)
                )),
        );
    }
    panel.into_any_element()
}

impl Documents {
    pub(super) fn review_proposal(
        &mut self,
        proposal_id: String,
        outcome: ProposalReview,
        cx: &mut Context<Self>,
    ) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some(proposal_id.clone());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let expected_proposal_id = proposal_id.clone();
        let work = shell::run(async move {
            let reviewed = shell
                .review_document_proposal(&document_id, &proposal_id, outcome)
                .await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((reviewed, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((updated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                if let Some(record) = detail
                                    .proposals
                                    .iter_mut()
                                    .find(|record| record.id == expected_proposal_id)
                                {
                                    *record = updated;
                                }
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                            view.order_rev = view.order_rev.wrapping_add(1);
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "Review was recorded, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice =
                                Some("Review recorded; filing is unchanged.".into());
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn regenerate_proposals(&mut self, cx: &mut Context<Self>) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some("regenerate".into());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let work = shell::run(async move {
            let generated = shell.regenerate_document_proposals(&document_id).await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((generated, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((mut generated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                detail.proposals.append(&mut generated);
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                            view.order_rev = view.order_rev.wrapping_add(1);
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "New proposals were saved, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice = Some(
                                "New rule run saved; earlier proposals and reviews remain.".into(),
                            );
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn generate_model_proposal(&mut self, cx: &mut Context<Self>) {
        if self.proposal_busy.is_some() {
            return;
        }
        let Some(document_id) = self
            .detail
            .value
            .as_ref()
            .map(|detail| detail.document.id.clone())
        else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_busy = Some("model".into());
        self.proposal_error = None;
        self.proposal_notice = None;
        let expected_document_id = document_id.clone();
        let work = shell::run(async move {
            let generated = shell.generate_document_model_proposal(&document_id).await?;
            let refreshed = shell.document_detail(&document_id).await;
            Ok((generated, refreshed))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() == Some(expected_document_id.as_str()) {
                    view.proposal_busy = None;
                    match result {
                        Ok((mut generated, refreshed)) => {
                            if let Some(detail) = &mut view.detail.value {
                                detail.proposals.append(&mut generated);
                            }
                            match refreshed {
                                Ok(detail) => {
                                    if let Some(rows) = &mut view.rows.value {
                                        if let Some(row) = rows
                                            .iter_mut()
                                            .find(|row| row.id == detail.document.id)
                                        {
                                            *row = detail.document.clone();
                                            view.order_rev = view.order_rev.wrapping_add(1);
                                        }
                                    }
                                    view.detail.settle(Ok(detail));
                                }
                                Err(error) => {
                                    view.proposal_error = Some(format!(
                                        "The model proposal was saved, but the workflow status did not refresh: {error}"
                                    ));
                                }
                            }
                            view.proposal_notice = Some(
                                "Local model proposal saved for review; filing remains separate.".into(),
                            );
                        }
                        Err(error) => view.proposal_error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn toggle_proposal_history(&mut self, cx: &mut Context<Self>) {
        self.show_proposal_history = !self.show_proposal_history;
        cx.notify();
    }

    pub(super) fn file_suggestion(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        let Some(category) = detail.document.suggested_category.clone() else {
            return;
        };
        let id = detail.document.id.clone();
        let Ok(shell) = self.repo.shell() else { return };
        let work = shell::run(async move { shell.review_document(&id, &category, None).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if let Ok(updated) = result {
                    if let Some(detail) = &mut view.detail.value {
                        if detail.document.id == updated.id {
                            detail.document = updated.clone();
                        }
                    }
                    if let Some(rows) = &mut view.rows.value {
                        if let Some(row) = rows.iter_mut().find(|row| row.id == updated.id) {
                            *row = updated;
                            view.order_rev = view.order_rev.wrapping_add(1);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn analyze_selected_document(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.workflow_busy.is_some() {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.workflow_busy = Some("analyze".into());
        self.workflow_error = None;
        cx.notify();
        let expected = id.clone();
        let work = shell::run(async move { shell.analyze_document(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() != Some(expected.as_str()) {
                    return;
                }
                view.workflow_busy = None;
                match result {
                    Ok(detail) => {
                        if let Some(rows) = &mut view.rows.value {
                            if let Some(row) =
                                rows.iter_mut().find(|row| row.id == detail.document.id)
                            {
                                *row = detail.document.clone();
                                view.order_rev = view.order_rev.wrapping_add(1);
                            }
                        }
                        view.detail.settle(Ok(detail));
                    }
                    Err(error) => view.workflow_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
