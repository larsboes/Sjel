//! Duplicate candidate detection, side-by-side comparison, and review actions.

use std::sync::Arc;

use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, Window};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use sjel_gpui_core::shell::{
    self, DuplicateCandidate, DuplicateCandidateKind, DuplicateDecision, DuplicateDecisionAction,
    DuplicatePageRange, ShellError,
};

use crate::data::Remote;
use crate::theme::Palette;

use super::viewer::{reader_page_element, Reader};
use super::Documents;

fn duplicate_filter_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    Button::new(id)
        .small()
        .label(if selected {
            format!("✓ {label}")
        } else {
            label.to_owned()
        })
        .on_click(on_click)
        .into_any_element()
}

pub(super) fn parse_page_ranges(value: &str) -> Result<Vec<DuplicatePageRange>, String> {
    let mut ranges = Vec::new();
    for part in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (first, last) = part
            .split_once('-')
            .ok_or_else(|| "Use inclusive ranges such as 1-3, 4-8.".to_owned())?;
        let first = first
            .trim()
            .parse::<u32>()
            .map_err(|_| "Page range start must be a positive number.".to_owned())?;
        let last = last
            .trim()
            .parse::<u32>()
            .map_err(|_| "Page range end must be a positive number.".to_owned())?;
        if first == 0 || last < first {
            return Err("Ranges must be positive and ordered, for example 1-3, 4-8.".into());
        }
        ranges.push(DuplicatePageRange { first, last });
    }
    if ranges.is_empty() {
        return Err("Enter every child range, for example 1-3, 4-8.".into());
    }
    Ok(ranges)
}

impl Documents {
    pub(super) fn load_duplicate_candidates(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.duplicate_page.loading = true;
        self.duplicate_error = None;
        let kind = self.duplicate_kind;
        let offset = self.duplicate_offset;
        let expected_kind = kind;
        let expected_offset = offset;
        let work = shell::run(async move { shell.duplicate_candidates(kind, offset, 50).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.duplicate_kind != expected_kind || view.duplicate_offset != expected_offset
                {
                    return;
                }
                view.duplicate_page.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_duplicate_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.duplicate_mode = enabled;
        if enabled {
            self.duplicate_offset = 0;
            self.duplicate_selection = None;
            self.load_duplicate_candidates(cx);
        }
        if !enabled {
            self.duplicate_selection = None;
            self.compare_readers = [Reader::default(), Reader::default()];
        }
        cx.notify();
    }

    pub(super) fn set_duplicate_kind(
        &mut self,
        kind: Option<DuplicateCandidateKind>,
        cx: &mut Context<Self>,
    ) {
        self.duplicate_kind = kind;
        self.duplicate_offset = 0;
        self.duplicate_page = Remote::default();
        self.duplicate_selection = None;
        self.compare_readers = [Reader::default(), Reader::default()];
        self.load_duplicate_candidates(cx);
    }

    pub(super) fn open_duplicate_candidate(
        &mut self,
        candidate: DuplicateCandidate,
        cx: &mut Context<Self>,
    ) {
        self.preferred_id = candidate
            .member_ids
            .first()
            .cloned()
            .filter(|_| candidate.member_ids.len() == 1);
        self.split_ranges.clear();
        self.split_input = None;
        self.duplicate_error = None;
        self.duplicate_selection = Some(candidate.clone());
        self.compare_readers = [Reader::default(), Reader::default()];
        for side in 0..2 {
            let Some(id) = candidate.member_ids.get(side).cloned() else {
                continue;
            };
            self.load_compare_original(side, id, cx);
        }
        cx.notify();
    }

    pub(super) fn load_compare_original(
        &mut self,
        side: usize,
        id: String,
        cx: &mut Context<Self>,
    ) {
        let Ok(shell) = self.repo.shell() else { return };
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        self.compare_readers[side] = Reader {
            id: Some(id.clone()),
            loading: true,
            ..Reader::default()
        };
        let expected_id = id.clone();
        let work = shell::run(async move { shell.document_original(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || view.compare_readers[side].id.as_deref() != Some(expected_id.as_str())
                {
                    return;
                }
                match result {
                    Ok(bytes) => {
                        view.compare_readers[side].bytes = Some(Arc::new(bytes));
                        view.compare_readers[side].loading = false;
                        view.ask_compare_page_count(side, cx);
                    }
                    Err(error) => {
                        view.compare_readers[side].loading = false;
                        view.compare_readers[side].error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn ask_compare_page_count(&mut self, side: usize, cx: &mut Context<Self>) {
        let Some(renderer) = self.renderer.as_ref() else {
            self.compare_readers[side].error = self.renderer_error.clone();
            return;
        };
        let (Some(bytes), Some(id)) = (
            self.compare_readers[side].bytes.clone(),
            self.compare_readers[side].id.clone(),
        ) else {
            return;
        };
        let answer = renderer.info(bytes);
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || view.compare_readers[side].id.as_deref() != Some(id.as_str())
                {
                    return;
                }
                match result {
                    Ok(pages) => view.compare_readers[side].pages = pages,
                    Err(error) => view.compare_readers[side].error = Some(error.to_string()),
                }
                view.request_compare_page(side, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_compare_page(&mut self, side: usize, cx: &mut Context<Self>) {
        let reader = &self.compare_readers[side];
        let (Some(renderer), Some(bytes), Some(id)) = (
            self.renderer.as_ref(),
            reader.bytes.clone(),
            reader.id.clone(),
        ) else {
            return;
        };
        let page = reader.page;
        let width = reader.zoom.width();
        if reader.drawn.contains_key(&(page, width)) {
            return;
        }
        let answer = renderer.render(bytes, page, width);
        let key = self
            .duplicate_selection
            .as_ref()
            .map(|item| item.candidate_key.clone());
        cx.spawn(async move |this, cx| {
            let Ok(result) = answer.await else { return };
            let _ = this.update(cx, |view, cx| {
                let reader = &mut view.compare_readers[side];
                if view
                    .duplicate_selection
                    .as_ref()
                    .map(|item| item.candidate_key.clone())
                    != key
                    || reader.id.as_deref() != Some(id.as_str())
                    || reader.page != page
                {
                    return;
                }
                match result {
                    Ok(image) => {
                        reader.drawn.insert((page, width), image);
                        reader.trim();
                    }
                    Err(error) => reader.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn move_compare_page(&mut self, side: usize, by: isize, cx: &mut Context<Self>) {
        if self.compare_readers[side].next_page(by) {
            self.request_compare_page(side, cx);
            cx.notify();
        }
    }

    pub(super) fn duplicate_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        let rows = self.rows.value.clone().unwrap_or_default();
        let mut body = div().flex_1().min_h_0().flex().flex_col().gap_3();
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(duplicate_filter_button(
                    "duplicate-all",
                    "All candidates",
                    self.duplicate_kind.is_none(),
                    cx.listener(|view, _, _, cx| view.set_duplicate_kind(None, cx)),
                ))
                .child(duplicate_filter_button(
                    "duplicate-exact",
                    "Exact copies",
                    self.duplicate_kind == Some(DuplicateCandidateKind::ExactCopy),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::ExactCopy), cx)
                    }),
                ))
                .child(duplicate_filter_button(
                    "duplicate-rescans",
                    "Rescans",
                    self.duplicate_kind == Some(DuplicateCandidateKind::Rescan),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::Rescan), cx)
                    }),
                ))
                .child(duplicate_filter_button(
                    "duplicate-parts",
                    "Possible parts",
                    self.duplicate_kind == Some(DuplicateCandidateKind::Part),
                    cx.listener(|view, _, _, cx| {
                        view.set_duplicate_kind(Some(DuplicateCandidateKind::Part), cx)
                    }),
                ))
                .child(div().flex_1())
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} · no files are changed",
                    self.duplicate_page
                        .value
                        .as_ref()
                        .map_or(0, |page| page.total)
                )),
        );

        if let Some(error) = &self.duplicate_error {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        if let Some(candidate) = self.duplicate_selection.clone() {
            let titles: Vec<_> = candidate
                .member_ids
                .iter()
                .take(2)
                .map(|id| {
                    rows.iter()
                        .find(|row| &row.id == id)
                        .map(|row| row.filename.clone())
                        .unwrap_or_else(|| id.chars().take(12).collect())
                })
                .collect();
            let label = match candidate.kind {
                DuplicateCandidateKind::ExactCopy => {
                    "Identical source bytes · SHA-256 match".to_owned()
                }
                DuplicateCandidateKind::Rescan => format!(
                    "Possible rescan · {:.0}% text overlap",
                    candidate.score.unwrap_or_default() * 100.0
                ),
                DuplicateCandidateKind::Part => format!(
                    "Possible part · {:.0}% text overlap",
                    candidate.score.unwrap_or_default() * 100.0
                ),
            };
            let mut panes = div().flex_1().min_h_0().flex().gap_3();
            for side in 0..2 {
                let reader = &self.compare_readers[side];
                let title = titles
                    .get(side)
                    .cloned()
                    .unwrap_or_else(|| "No second document record".into());
                let title = candidate.part.as_ref().map_or(title.clone(), |part| {
                    match candidate.member_ids.get(side).map(String::as_str) {
                        Some(id) if id == part.part_id => format!("Likely part · {title}"),
                        Some(id) if id == part.parent_id => format!("Likely parent · {title}"),
                        _ => title,
                    }
                });
                let id = reader.id.clone();
                let page = reader.page;
                let pages = reader.pages;
                let chosen = id
                    .as_ref()
                    .is_some_and(|id| self.preferred_id.as_ref() == Some(id));
                let pane = div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title),
                            )
                            .when_some(id.clone(), |bar, id| {
                                bar.child(
                                    Button::new(format!("duplicate-prefer-{side}"))
                                        .small()
                                        .label(if chosen { "Preferred" } else { "Prefer" })
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.preferred_id = Some(id.clone());
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(format!("duplicate-prev-{side}"))
                                    .small()
                                    .label("‹")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.move_compare_page(side, -1, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .w(px(76.))
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(if pages == 0 {
                                        "—".into()
                                    } else {
                                        format!("{} / {}", page + 1, pages)
                                    }),
                            )
                            .child(
                                Button::new(format!("duplicate-next-{side}"))
                                    .small()
                                    .label("›")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.move_compare_page(side, 1, cx)
                                    })),
                            ),
                    )
                    .child(reader_page_element(reader, palette));
                panes = panes.child(pane);
            }
            let receipts = candidate
                .locations
                .iter()
                .map(|location| {
                    location
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&location.path)
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(" · ");
            body = body
                .child(div().text_sm().text_color(rgb(palette.amber)).child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} recorded source location(s): {}",
                            candidate.locations.len(),
                            if receipts.is_empty() {
                                "no source receipt"
                            } else {
                                &receipts
                            }
                        )),
                )
                .child(panes);

            if candidate.kind != DuplicateCandidateKind::Part && candidate.member_ids.len() > 2 {
                let mut members = div().flex().items_center().gap_2().flex_wrap();
                for (index, id) in candidate.member_ids.iter().enumerate() {
                    let filename = rows
                        .iter()
                        .find(|row| &row.id == id)
                        .map(|row| row.filename.clone())
                        .unwrap_or_else(|| id.chars().take(12).collect());
                    let preferred = self.preferred_id.as_ref() == Some(id);
                    let id = id.clone();
                    members = members.child(
                        Button::new(format!("duplicate-member-{index}"))
                            .small()
                            .label(if preferred {
                                format!("✓ {filename}")
                            } else {
                                filename
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.preferred_id = Some(id.clone());
                                cx.notify();
                            })),
                    );
                }
                body = body.child(members);
            }

            if candidate.kind == DuplicateCandidateKind::Part {
                let input = self.split_range_field(window, cx);
                let parent = candidate.part.as_ref();
                let parent_pages = parent
                    .and_then(|part| {
                        self.compare_readers
                            .iter()
                            .find(|reader| reader.id.as_deref() == Some(part.parent_id.as_str()))
                            .map(|reader| reader.pages)
                    })
                    .unwrap_or(0);
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Split page ranges · parent has {parent_pages} pages"
                        ))
                        .child(div().w(px(220.)).child(Input::new(&input).small()))
                        .when(parent_pages > 0, |bar| {
                            let parent_id = parent
                                .map(|part| part.parent_id.clone())
                                .unwrap_or_default();
                            bar.child(
                                Button::new("duplicate-record-split")
                                    .small()
                                    .label("Record split relationship")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        match parse_page_ranges(&view.split_ranges) {
                                            Ok(ranges) => view.record_duplicate_decision(
                                                DuplicateDecisionAction::Split {
                                                    parent_id: parent_id.clone(),
                                                    page_count: u32::try_from(parent_pages)
                                                        .unwrap_or(u32::MAX),
                                                    ranges,
                                                },
                                                cx,
                                            ),
                                            Err(error) => {
                                                view.duplicate_error = Some(error);
                                                cx.notify();
                                            }
                                        }
                                    })),
                            )
                        })
                        .child(
                            Button::new("duplicate-keep-separate")
                                .small()
                                .label("Keep separate")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.record_duplicate_decision(
                                        DuplicateDecisionAction::Dismiss,
                                        cx,
                                    )
                                })),
                        ),
                );
            } else {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(self.preferred_id.is_some(), |bar| {
                            bar.child(
                                Button::new("duplicate-confirm-group")
                                    .small()
                                    .label("Confirm group")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        if let Some(survivor_id) = view.preferred_id.clone() {
                                            view.record_duplicate_decision(
                                                DuplicateDecisionAction::ConfirmGroup {
                                                    survivor_id,
                                                },
                                                cx,
                                            );
                                        }
                                    })),
                            )
                        })
                        .child(
                            Button::new("duplicate-keep-separate")
                                .small()
                                .label("Keep separate")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.record_duplicate_decision(
                                        DuplicateDecisionAction::Dismiss,
                                        cx,
                                    )
                                })),
                        ),
                );
            }
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("duplicate-back-queue")
                            .small()
                            .label("Back to candidates")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.duplicate_selection = None;
                                view.compare_readers = [Reader::default(), Reader::default()];
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                        "Source records stay intact; approved page splits are stored as child PDFs in Sjel.",
                    )),
            );
        } else {
            if self.duplicate_page.loading && self.duplicate_page.value.is_none() {
                body = body.child(
                    div()
                        .py_5()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Finding duplicate candidates…"),
                );
            }
            if let Some(error) = &self.duplicate_page.error {
                body = body.child(
                    div()
                        .py_3()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error.to_string()),
                );
            }
            let candidates = self
                .duplicate_page
                .value
                .as_ref()
                .map(|page| page.candidates.clone())
                .unwrap_or_default();
            if candidates.is_empty()
                && !self.duplicate_page.loading
                && self.duplicate_page.error.is_none()
            {
                body = body.child(
                    div()
                        .py_5()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No unresolved candidates in this filter."),
                );
            }
            for (candidate_index, candidate) in candidates.into_iter().enumerate() {
                let candidate_for_click = candidate.clone();
                let names = candidate
                    .member_ids
                    .iter()
                    .take(2)
                    .map(|id| {
                        rows.iter()
                            .find(|row| &row.id == id)
                            .map(|row| row.filename.clone())
                            .unwrap_or_else(|| id.chars().take(12).collect::<String>())
                    })
                    .collect::<Vec<_>>();
                let evidence = match candidate.kind {
                    DuplicateCandidateKind::ExactCopy => "Exact SHA-256 match".to_owned(),
                    DuplicateCandidateKind::Rescan => format!(
                        "Possible rescan · {:.0}%",
                        candidate.score.unwrap_or_default() * 100.0
                    ),
                    DuplicateCandidateKind::Part => format!(
                        "Possible part · {:.0}%",
                        candidate.score.unwrap_or_default() * 100.0
                    ),
                };
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded(px(5.))
                        .child(
                            Button::new(format!("duplicate-candidate-{candidate_index}"))
                                .small()
                                .label(names.join("  ↔  "))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_duplicate_candidate(candidate_for_click.clone(), cx)
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(
                                    if candidate.kind == DuplicateCandidateKind::ExactCopy {
                                        palette.green
                                    } else {
                                        palette.amber
                                    },
                                ))
                                .child(evidence),
                        ),
                );
            }
            let offset = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(0, |page| page.offset);
            let total = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(0, |page| page.total);
            let limit = self
                .duplicate_page
                .value
                .as_ref()
                .map_or(50, |page| page.limit);
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .when(offset > 0, |bar| {
                        bar.child(
                            Button::new("duplicate-prev-batch")
                                .small()
                                .label("Previous")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.duplicate_offset =
                                        view.duplicate_offset.saturating_sub(50);
                                    view.load_duplicate_candidates(cx);
                                })),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{}–{} of {}",
                                if total == 0 { 0 } else { offset + 1 },
                                (offset + limit).min(total),
                                total
                            )),
                    )
                    .when(offset + limit < total, |bar| {
                        bar.child(
                            Button::new("duplicate-next-batch")
                                .small()
                                .label("Next")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.duplicate_offset += 50;
                                    view.load_duplicate_candidates(cx);
                                })),
                        )
                    }),
            );
        }
        body.into_any_element()
    }

    pub(super) fn record_duplicate_decision(
        &mut self,
        action: DuplicateDecisionAction,
        cx: &mut Context<Self>,
    ) {
        let Some(candidate) = self.duplicate_selection.clone() else {
            return;
        };
        let Ok(shell) = self.repo.shell() else { return };
        let decision = DuplicateDecision {
            candidate_key: candidate.candidate_key,
            kind: candidate.kind,
            member_ids: candidate.member_ids,
            action,
        };
        let work = shell::run(async move { shell.review_duplicate(decision).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(()) => {
                        view.duplicate_selection = None;
                        view.compare_readers = [Reader::default(), Reader::default()];
                        view.duplicate_offset = 0;
                        view.load_duplicate_candidates(cx);
                    }
                    Err(error) => view.duplicate_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
