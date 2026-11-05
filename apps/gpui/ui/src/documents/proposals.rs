//! Document proposal reviews, rule & model proposal generations, and document splitting.

use std::collections::BTreeMap;

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, Window};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::shell::{
    self, DocumentDetail, DocumentSplitChildInput, DocumentSplitRequest, ProposalEvidenceSource,
    ProposalOutcome, ProposalReview, ProposalSource, ShellError,
};

use crate::theme::Palette;

use super::duplicates::parse_page_ranges;
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
                    Button::new("document-regenerate-proposals")
                        .small()
                        .ghost()
                        .label(if view.proposal_busy.as_deref() == Some("regenerate") {
                            "Saving…"
                        } else {
                            "Regenerate rules"
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|view, _, _, cx| view.regenerate_proposals(cx))),
                )
                .child(
                    Button::new("document-model-proposal")
                        .small()
                        .ghost()
                        .label(if view.proposal_busy.as_deref() == Some("model") {
                            "Asking local model…"
                        } else {
                            "Ask local model"
                        })
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
            Button::new("document-toggle-proposal-history")
                .small()
                .ghost()
                .label(if view.show_proposal_history {
                    "Show latest run only"
                } else {
                    "Show earlier runs"
                })
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
                            Button::new(format!("document-proposal-agree-{}", record.id))
                                .xsmall()
                                .ghost()
                                .label("Agree")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        proposal_id.clone(),
                                        ProposalReview::Accepted,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new(format!("document-proposal-reject-{}", source_id))
                                .xsmall()
                                .ghost()
                                .label("Reject")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        source_id.clone(),
                                        ProposalReview::Rejected,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new(format!("document-proposal-override-{}", override_id))
                                .xsmall()
                                .ghost()
                                .label("Override")
                                .disabled(busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.review_proposal(
                                        override_id.clone(),
                                        ProposalReview::Overridden,
                                        cx,
                                    )
                                })),
                        ),
                )
        }))
        .into_any_element()
}

pub(super) fn workflow_panel(
    view: &Documents,
    detail: &DocumentDetail,
    split_input: &Entity<InputState>,
    palette: Palette,
    cx: &mut Context<Documents>,
) -> AnyElement {
    let document = &detail.document;
    let is_pdf = document.media_type == "application/pdf";
    let is_original = document.parent_id.is_none();
    let mut panel = div().flex().flex_col().gap_1();

    if is_pdf {
        let analyzing = view.workflow_busy.as_deref() == Some("analyze");
        panel = panel.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Button::new("document-analyze-pages")
                        .small()
                        .label(if analyzing {
                            "Analyzing locally…"
                        } else if document.page_count == 0 {
                            "Analyze pages locally"
                        } else {
                            "Refresh page analysis"
                        })
                        .on_click(cx.listener(|view, _, _, cx| view.analyze_selected_document(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} analyzed page(s)", document.page_count)),
                ),
        );

        if let Some(suggestion) = detail.split_suggestions.as_ref() {
            if suggestion.boundaries.is_empty() {
                if let Some(reason) = suggestion.abstention.as_ref() {
                    panel = panel.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("No automatic cut: {reason}")),
                    );
                }
            } else {
                let cuts = suggestion
                    .boundaries
                    .iter()
                    .map(|boundary| boundary.before_page.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                panel = panel.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("Page-one markers suggest cuts before pages {cuts}. Review each page before splitting.")),
                );
                for boundary in &suggestion.boundaries {
                    panel = panel.child(div().text_xs().text_color(rgb(palette.muted)).child(
                        format!(
                            "Page {} · {} · {}",
                            boundary.before_page, boundary.marker, boundary.quote
                        ),
                    ));
                }
            }
        }

        if is_original && document.page_count > 1 {
            let splitting = view.workflow_busy.as_deref() == Some("split");
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mt(px(2.))
                    .child(div().w(px(220.)).child(Input::new(split_input).small()))
                    .child(
                        Button::new("document-create-split")
                            .small()
                            .label(if splitting {
                                "Creating children…"
                            } else {
                                "Split into records"
                            })
                            .on_click(
                                cx.listener(|view, _, _, cx| view.split_selected_document(cx)),
                            ),
                    )
                    .when_else(
                        detail
                            .split_suggestions
                            .as_ref()
                            .is_some_and(|suggestion| !suggestion.ranges.is_empty()),
                        |row| {
                            row.child(
                                Button::new("document-use-split-suggestions")
                                    .small()
                                    .ghost()
                                    .label("Use page-marker suggestion")
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.use_split_suggestions(window, cx)
                                    })),
                            )
                        },
                        |row| row,
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Use every page once, e.g. 1-2,3-5. Original stays intact."),
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
                    Button::new(format!("document-child-{}", &child.id[..12]))
                        .small()
                        .label(label)
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
    if is_pdf && !is_original {
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
    pub(super) fn split_range_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.split_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("1-3, 4-8"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                view.split_ranges = input.read(cx).value().trim().to_owned();
                cx.notify();
            }
        });
        self.split_input = Some((input.clone(), subscription));
        input
    }

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

    pub(super) fn split_selected_document(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.workflow_busy.is_some() {
            return;
        }
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        if detail.document.parent_id.is_some() || detail.document.page_count < 2 {
            return;
        }
        let page_count = detail.document.page_count;
        let filename = detail.document.filename.clone();
        let ranges = match parse_page_ranges(&self.split_ranges) {
            Ok(ranges) => ranges,
            Err(error) => {
                self.workflow_error = Some(error);
                cx.notify();
                return;
            }
        };
        let stem = std::path::Path::new(&filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Document");
        let request = DocumentSplitRequest {
            page_count: u32::try_from(page_count).unwrap_or(u32::MAX),
            children: ranges
                .into_iter()
                .map(|range| DocumentSplitChildInput {
                    first: range.first,
                    last: range.last,
                    filename: format!("{stem} pages {}-{}.pdf", range.first, range.last),
                })
                .collect(),
        };
        let Ok(shell) = self.repo.shell() else { return };
        self.workflow_busy = Some("split".into());
        self.workflow_error = None;
        cx.notify();
        let expected = id.clone();
        let refresh_id = id.clone();
        let work = shell::run(async move {
            let children = shell.split_document(&id, &request).await?;
            let parent = shell.document_detail(&refresh_id).await;
            Ok((children, parent))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.selected.as_deref() != Some(expected.as_str()) {
                    return;
                }
                view.workflow_busy = None;
                match result {
                    Ok((children, parent)) => {
                        if let Some(detail) = &mut view.detail.value {
                            detail.document.workflow_stage = "split".into();
                            for child in &children {
                                if !detail
                                    .children
                                    .iter()
                                    .any(|existing| existing.id == child.id)
                                {
                                    detail.children.push(child.clone());
                                }
                            }
                        }
                        if let Some(rows) = &mut view.rows.value {
                            for child in children {
                                if !rows.iter().any(|existing| existing.id == child.id) {
                                    rows.push(child);
                                }
                            }
                        }
                        match parent {
                            Ok(detail) => {
                                if let Some(rows) = &mut view.rows.value {
                                    if let Some(row) = rows
                                        .iter_mut()
                                        .find(|row| row.id == detail.document.id)
                                    {
                                        *row = detail.document.clone();
                                    }
                                }
                                view.detail.settle(Ok(detail));
                            }
                            Err(error) => {
                                view.workflow_error = Some(format!(
                                    "Split was saved, but the parent workflow status did not refresh: {error}"
                                ));
                            }
                        }
                    }
                    Err(error) => view.workflow_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn use_split_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.value.as_ref() else {
            return;
        };
        let Some(suggestion) = detail.split_suggestions.as_ref() else {
            return;
        };
        if suggestion.ranges.is_empty() {
            return;
        }
        let value = suggestion
            .ranges
            .iter()
            .map(|range| format!("{}-{}", range.first, range.last))
            .collect::<Vec<_>>()
            .join(", ");
        self.split_ranges = value.clone();
        if let Some((input, _)) = &self.split_input {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        cx.notify();
    }
}
