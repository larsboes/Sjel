use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString, Window,
};
use sjel_gpui_core::shell::{FeedContentDetail, FeedEntry};

use crate::doc_view::render_markdown;
use crate::primitives::{filter_pill, virtual_list};
use crate::theme::{tint, Density, Palette};

use super::actions::{Back, Dismiss, Keep, OpenOriginal};
use super::types::{Filter, Tab};
use super::Feed;

impl Feed {
    pub(crate) fn list(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.sync_reading();
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

        let mut column = div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(self.title_row(cx))
            .child(muted(
                "j/k move · s keep · d dismiss · u undo · o read",
                palette,
            ))
            .child(toolbar);
        if self.rows.value.is_none() && self.rows.loading {
            column = column.child(muted("Loading the feed…", palette));
        }
        if let Some(error) = &self.rows.error {
            column = column.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        if self.reading_order.is_empty() && !self.rows.loading && self.rows.error.is_none() {
            column = column.child(muted(
                match self.filter {
                    Filter::New => "Nothing new. The inbox is clear.",
                    Filter::Kept => "Nothing kept in the last 30 days.",
                    Filter::Dismissed => "Nothing dismissed in the last 30 days.",
                },
                palette,
            ));
        }
        if !self.reading_order.is_empty() {
            // One row per item on screen: the list asks for the rows it draws, not for all of them.
            let entity = cx.entity();
            column = column.child(virtual_list(
                "feed-reading",
                &self.reading_list,
                move |ix, _window, cx| {
                    entity.update(cx, |feed, cx| feed.reading_row(ix, palette, cx))
                },
            ));
        }
        column.into_any_element()
    }

    /// One Reading row: the day header when the row opens a day, then the entry itself.
    pub(crate) fn reading_row(
        &self,
        ix: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(rows) = self.rows.value.as_ref() else {
            return div().into_any_element();
        };
        let Some(entry) = self
            .reading_order
            .get(ix)
            .and_then(|index| rows.get(*index))
        else {
            return div().into_any_element();
        };
        let day_header = match ix.checked_sub(1) {
            None => true,
            Some(previous) => {
                self.reading_order
                    .get(previous)
                    .and_then(|index| rows.get(*index))
                    .map(|row| row.day.as_str())
                    != Some(entry.day.as_str())
            }
        };
        let id = entry.id.clone();
        let chosen = self.cursor.as_deref() == Some(entry.id.as_str());
        let preview = entry
            .summary
            .as_deref()
            .or(entry.digest_preview.as_deref())
            .unwrap_or_default()
            .trim()
            .to_owned();
        let score = entry.score();
        let score_badge = (score > 0.).then(|| {
            let color = if score >= 0.8 {
                palette.green
            } else if score >= 0.5 {
                palette.amber
            } else {
                palette.muted
            };
            div()
                .px_1p5()
                .py(px(0.5))
                .rounded(px(3.))
                .bg(rgba(tint(color, 0x18)))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(color))
                .child(format!("{:.0}%", score * 100.))
        });
        let keep_id = entry.id.clone();
        let dismiss_id = entry.id.clone();
        let quick_triage = (entry.status == "new").then(|| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id(("feed-quick-keep", ix))
                        .px_2()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(rgba(tint(palette.green, 0x18)))
                        .hover(move |style| style.bg(rgba(tint(palette.green, 0x33))))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.green))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.decide(keep_id.clone(), "keeper", cx);
                        }))
                        .child("Keep"),
                )
                .child(
                    div()
                        .id(("feed-quick-dismiss", ix))
                        .px_2()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(rgba(tint(palette.red, 0x18)))
                        .hover(move |style| style.bg(rgba(tint(palette.red, 0x33))))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.red))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.decide(dismiss_id.clone(), "dismissed", cx);
                        }))
                        .child("Dismiss"),
                )
        });
        let row = div()
            .id(("feed-row", ix))
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
                    .children(score_badge)
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(meta(entry)),
                    )
                    .children(quick_triage),
            )
            .when(!preview.is_empty(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .line_clamp(2)
                        .child(preview),
                )
            });
        let mut item = div().flex().flex_col().gap_1().pb(px(4.));
        if day_header {
            item = item.child(
                div()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child(entry.day.clone()),
            );
        }
        item.child(row).into_any_element()
    }

    pub(crate) fn reader(
        &self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(source) = self.content_source {
            return self.source_reader(source, palette, window, cx);
        }
        let back = Button::new("feed-back")
            .small()
            .ghost()
            .label(match self.tab {
                Tab::Review => "← Review",
                Tab::Library => "← Library",
                _ => "← Feed",
            })
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
        let content_detail = self.content_detail.value.as_ref();
        let digest = content_detail
            .and_then(|content| content.digest.as_ref())
            .and_then(|digest| digest.text.as_deref())
            .filter(|text| !text.trim().is_empty())
            .filter(|text| detail.summary.as_deref() != Some(*text))
            .filter(|text| detail.transcript.as_deref() != Some(*text));
        let source_content = content_detail
            .and_then(|content| content.content.as_deref())
            .filter(|text| !text.trim().is_empty())
            .filter(|text| detail.transcript.as_deref() != Some(*text));
        let mut reader = div()
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
            .when_some(digest, |el, digest| {
                el.child(section("LOCAL DIGEST", digest.to_owned(), palette))
            })
            .children(text("TRANSCRIPT", &detail.transcript))
            .when_some(source_content, |el, body| {
                let label = content_detail
                    .map(|content| content.content_label.as_str())
                    .filter(|label| !label.trim().is_empty())
                    .unwrap_or("SOURCE CONTENT");
                el.child(section(label, body.to_owned(), palette))
            })
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
            );
        if let Some(content) = content_detail {
            reader = reader.child(self.cloud_processing_panel(content, palette, cx));
        }
        reader.into_any_element()
    }

    pub(crate) fn source_reader(
        &self,
        source: &str,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mail = source == "mail";
        let back = Button::new("source-content-back")
            .small()
            .ghost()
            .label(if mail { "← Mail" } else { "← Feed" })
            .on_click(cx.listener(|view, _, window, cx| view.on_back(&Back, window, cx)));
        let Some(item) = self.content_detail.value.as_ref() else {
            let state = match &self.content_detail.error {
                Some(error) => div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
                None => muted("Opening source item…", palette),
            };
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(back)
                .child(state)
                .into_any_element();
        };

        let open = Button::new("source-content-original")
            .small()
            .label(if mail {
                "Open in Gmail ↗"
            } else {
                "Open in Calendar ↗"
            })
            .on_click(
                cx.listener(|view, _, window, cx| view.on_open_original(&OpenOriginal, window, cx)),
            );
        let edit = (!mail).then(|| {
            Button::new("calendar-edit")
                .small()
                .label(if self.calendar_edit.is_some() {
                    "Editing…"
                } else {
                    "Edit"
                })
                .disabled(self.calendar_edit.as_ref().is_some_and(|form| form.saving))
                .on_click(cx.listener(|view, _, window, cx| {
                    if view.calendar_edit.is_some() {
                        view.calendar_edit = None;
                        cx.notify();
                    } else {
                        view.open_calendar_edit(window, cx);
                    }
                }))
        });
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(back)
            .child(div().flex_1())
            .children(edit)
            .child(open);
        let mut reader = div()
            .flex()
            .flex_col()
            .gap_4()
            .max_w(px(760.))
            .child(actions)
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title(item.title.as_deref(), &item.url)),
            );

        if let Some(editor) = self.calendar_edit_panel(palette, window, cx) {
            reader = reader.child(editor);
        }

        let metadata = [
            Some(item.kind.as_str()),
            item.author.as_deref(),
            (!item.day.is_empty()).then_some(item.day.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        if !metadata.is_empty() {
            reader = reader.child(muted(&metadata, palette));
        }
        if let Some(calendar) = item.calendar.as_ref() {
            let when = if calendar.all_day {
                format!(
                    "All day · {} through the day before {}",
                    calendar.starts_at, calendar.ends_at
                )
            } else {
                format!("{} – {}", calendar.starts_at, calendar.ends_at)
            };
            reader = reader.child(section("WHEN", when, palette));
            if let Some(location) = calendar
                .location
                .as_deref()
                .filter(|location| !location.trim().is_empty())
            {
                reader = reader.child(section("LOCATION", location.to_owned(), palette));
            }
            if !calendar.commitment.is_empty() {
                reader = reader.child(section("COMMITMENT", calendar.commitment.clone(), palette));
            }
            let export_blocked = if calendar.entry_source == "google" {
                Some("Google entries cannot be exported back to Google.")
            } else if calendar.rhythm_id.is_some() {
                Some("Rhythm instances cannot be exported individually.")
            } else {
                None
            };
            let exported = self
                .calendar_exports
                .value
                .as_ref()
                .is_some_and(|entries| entries.iter().any(|entry| entry.entry_id == item.id));
            let export_id = item.id.clone();
            let export_button = Button::new("calendar-google-export-opt-in")
                .small()
                .when(exported, |button| button.ghost())
                .when(!exported, |button| button.primary())
                .label(if self.calendar_export_busy {
                    "Saving…"
                } else if exported {
                    "Remove Google export opt-in"
                } else {
                    "Allow Google export"
                })
                .disabled(
                    self.calendar_export_busy
                        || export_blocked.is_some()
                        || self.calendar_exports.value.is_none(),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.set_calendar_export(export_id.clone(), !exported, cx)
                }));
            let can_preview = self
                .calendar_exports
                .value
                .as_ref()
                .is_some_and(|entries| !entries.is_empty());
            let preview_button = Button::new("calendar-google-export-preview")
                .small()
                .ghost()
                .label(if self.calendar_export_report.loading {
                    "Building preview…"
                } else {
                    "Preview approved exports"
                })
                .disabled(self.calendar_export_busy || !can_preview)
                .on_click(cx.listener(|view, _, _, cx| view.preview_calendar_export(cx)));
            reader = reader.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(export_button)
                    .child(preview_button),
            );
            if let Some(report) = self.calendar_export_report.value.as_ref() {
                reader = reader.child(
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .bg(rgba(palette.raised))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(if report.dry_run {
                            format!("{} approved entries would be sent", report.pushed.len())
                        } else {
                            format!("Export finished: {} created, {} updated", report.inserted, report.patched)
                        }))
                        .children(report.pushed.iter().map(|entry| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} · {}", entry.title, entry.operation))
                        }))
                        .children(report.skipped.iter().map(|entry| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .child(format!("Skipped · {}", entry.reason))
                        }))
                        .when(report.dry_run && report.pushed.is_empty(), |panel| {
                            panel.child(muted("No approved entries are ready to send.", palette))
                        })
                        .when(
                            report.dry_run
                                && !self.calendar_export_confirm
                                && !report.pushed.is_empty(),
                            |panel| {
                            panel.child(
                                Button::new("calendar-google-export-run")
                                    .small()
                                    .primary()
                                    .label(if self.calendar_export_busy {
                                        "Sending…".to_owned()
                                    } else {
                                        format!("Send {} approved entries to Google", report.pushed.len())
                                    })
                                    .disabled(self.calendar_export_busy)
                                    .on_click(cx.listener(|view, _, _, cx| view.request_calendar_export(cx))),
                            )
                        })
                        .when(
                            report.dry_run
                                && self.calendar_export_confirm
                                && !report.pushed.is_empty(),
                            |panel| {
                                panel.child(muted(
                                    "This sends only the entries individually approved for Google.",
                                    palette,
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            Button::new("calendar-google-export-cancel")
                                                .small()
                                                .ghost()
                                                .label("Cancel")
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.calendar_export_confirm = false;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("calendar-google-export-confirm")
                                                .small()
                                                .primary()
                                                .label(format!("Confirm send · {} entries", report.pushed.len()))
                                                .disabled(self.calendar_export_busy)
                                                .on_click(cx.listener(|view, _, _, cx| {
                                                    view.run_calendar_export(cx)
                                                })),
                                        ),
                                )
                            },
                        ),
                );
            }
            if let Some(reason) = export_blocked {
                reader = reader.child(muted(reason, palette));
            }
            if let Some(error) = self
                .calendar_export_error
                .as_ref()
                .or(self.calendar_exports.error.as_ref())
                .or(self.calendar_export_report.error.as_ref())
            {
                reader = reader.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(format!("Google Calendar action failed: {error}")),
                );
            }
        }
        if let Some(summary) = item
            .summary
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            reader = reader.child(section("SUMMARY", summary.to_owned(), palette));
        }
        if let Some(digest) = item
            .digest
            .as_ref()
            .and_then(|digest| digest.text.as_deref())
            .filter(|text| !text.trim().is_empty())
        {
            reader = reader.child(section("LOCAL DIGEST", digest.to_owned(), palette));
        }
        if matches!(source, "feed" | "mail") {
            reader = reader.child(self.cloud_processing_panel(item, palette, cx));
        }
        if let Some(content) = item
            .content
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            let label = if item.content_label.trim().is_empty() {
                if mail {
                    "MAIL PREVIEW"
                } else {
                    "NOTES"
                }
            } else {
                item.content_label.as_str()
            };
            reader = reader.child(section(label, content.to_owned(), palette));
        } else if item.content_status == "none" {
            reader = reader.child(muted(
                if mail {
                    "Sjel holds no mail text for this item. Open it in Gmail."
                } else {
                    "This calendar entry has no notes."
                },
                palette,
            ));
        }
        reader.into_any_element()
    }

    pub(crate) fn cloud_processing_panel(
        &self,
        item: &FeedContentDetail,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let status = match item.cloud_processing.status.as_str() {
            "staged" => "Staged locally",
            "stale" => "Review again",
            _ => "Not prepared",
        };
        let preview_button = Button::new("feed-cloud-preview")
            .small()
            .label(if self.cloud_preview_busy {
                "Preparing locally…"
            } else if item.cloud_processing.status == "staged" {
                "Review a fresh copy"
            } else {
                "Preview cloud-ready copy"
            })
            .disabled(self.cloud_preview_busy || self.cloud_approval_busy)
            .on_click(cx.listener(|view, _, _, cx| view.prepare_cloud_preview(cx)));
        let mut panel = div()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.raised))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Cloud processing"),
                    )
                    .child(muted(status, palette)),
            )
            .child(muted(
                "Build a bounded copy locally and inspect it before staging approval.",
                palette,
            ))
            .child(preview_button);

        if let Some(error) = self
            .cloud_preview_error
            .as_ref()
            .or(self.cloud_preview.error.as_ref())
        {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(format!("Cloud preview failed: {error}")),
            );
        }

        if let Some(preview) = self.cloud_preview.value.as_ref() {
            let can_approve = preview.approval_required && preview.provider_calls == 0;
            let mut review = div()
                .mt_2()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(palette.border))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Review the cloud-ready copy"),
                )
                .child(muted(
                    &format!(
                        "{} source → {} derivative · {} redactions · {} · {}",
                        preview.original_data_class,
                        preview.derivative_data_class,
                        preview.redaction_count,
                        if preview.truncated {
                            "bounded at 16,000 characters"
                        } else {
                            "complete bounded document"
                        },
                        preview.entity_detection
                    ),
                    palette,
                ));
            if let Some(receipt) = preview.redaction_receipt.as_deref() {
                review = review.child(section("REDACTION RECEIPT", receipt.to_owned(), palette));
            }
            if !preview.redactions.is_empty() {
                review = review
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Detected locally"),
                    )
                    .children(preview.redactions.iter().map(|finding| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} {} × {}",
                                finding.marker, finding.entity_type, finding.count
                            ))
                    }));
            }
            review = review.child(
                div()
                    .id("feed-cloud-preview-document")
                    .max_h(px(260.))
                    .overflow_y_scroll()
                    .track_scroll(&self.cloud_preview_scroll)
                    .vertical_scrollbar(&self.cloud_preview_scroll)
                    .p_2()
                    .rounded(px(4.))
                    .bg(rgba(palette.surface))
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(18.))
                            .child(preview.document.clone()),
                    ),
            );
            if !preview.limitations.is_empty() {
                review = review
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Review limits"),
                    )
                    .children(preview.limitations.iter().map(|limitation| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("• {limitation}"))
                    }));
            }
            review = review
                .child(muted(
                    &format!("This stores the exact reviewed copy locally. Provider calls: {}.", preview.provider_calls),
                    palette,
                ))
                .when(!can_approve, |el| {
                    el.child(muted(
                        "This preview cannot be approved because it does not meet the local approval contract.",
                        palette,
                    ))
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("feed-cloud-preview-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(self.cloud_approval_busy)
                                .on_click(cx.listener(|view, _, _, cx| view.close_cloud_preview(cx))),
                        )
                        .child(
                            Button::new("feed-cloud-preview-approve")
                                .small()
                                .primary()
                                .label(if self.cloud_approval_busy {
                                    "Staging…"
                                } else {
                                    "Approve cloud-ready copy"
                                })
                                .disabled(!can_approve || self.cloud_approval_busy)
                                .on_click(cx.listener(|view, _, _, cx| view.approve_cloud_preview(cx))),
                        ),
                );
            panel = panel.child(review);
        } else if self.cloud_preview.loading {
            panel = panel.child(muted("Building a local preview…", palette));
        }

        if item.cloud_processing.status == "staged" {
            let dispatch = match item.cloud_processing.dispatch_status.as_str() {
                "queued" => "Queued",
                "running" => "Running",
                "succeeded" => "Completed",
                "failed" => "Failed",
                _ => "Ready to queue",
            };
            let mut job_box = div()
                .mt_2()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(palette.border))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("CLOUD JOB"),
                        )
                        .child(muted(dispatch, palette)),
                );

            if let Some(role) = item.cloud_processing.provider_role.as_deref() {
                job_box = job_box.child(muted(
                    &format!(
                        "Provider role: {role} · Calls used: {}",
                        item.cloud_processing.provider_calls
                    ),
                    palette,
                ));
            }

            if let Some(providers) = self.cloud_providers.value.as_ref() {
                if providers.is_empty() {
                    job_box = job_box.child(muted(
                        "No reviewed HTTPS cloud roles are configured.",
                        palette,
                    ));
                } else {
                    let mut role_buttons = div().flex().flex_wrap().gap_2();
                    for provider in providers {
                        let role = provider.role.clone();
                        let remaining = provider
                            .requests_remaining_today
                            .map(|r| format!(" · {r} left"))
                            .unwrap_or_default();
                        let label = if provider.available {
                            format!(
                                "Queue {} ({}{})",
                                provider.provider_label, provider.data_tier, remaining
                            )
                        } else {
                            format!(
                                "{} unavailable ({})",
                                provider.provider_label,
                                provider
                                    .unavailable_reason
                                    .as_deref()
                                    .unwrap_or("unavailable")
                            )
                        };
                        role_buttons = role_buttons.child(
                            Button::new(SharedString::from(format!("feed-cloud-queue-{role}")))
                                .small()
                                .label(label)
                                .disabled(!provider.available || self.cloud_job_busy)
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.queue_cloud_job(role.clone(), cx);
                                })),
                        );
                    }
                    job_box = job_box.child(role_buttons);
                }
            }

            if item.cloud_processing.job_id.is_some()
                && item.cloud_processing.dispatch_status != "succeeded"
            {
                job_box = job_box.child(
                    div().flex().child(
                        Button::new("feed-cloud-job-run")
                            .small()
                            .primary()
                            .label(if self.cloud_job_busy {
                                "Running cloud job…"
                            } else {
                                "Run cloud job"
                            })
                            .disabled(self.cloud_job_busy)
                            .on_click(cx.listener(|view, _, _, cx| view.run_cloud_job(cx))),
                    ),
                );
            }

            if let Some(error) = self
                .cloud_job_error
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| item.cloud_processing.last_error.clone())
            {
                job_box = job_box.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.red))
                        .child(format!("Cloud job error: {error}")),
                );
            }

            if let Some(result) = item.cloud_processing.result.as_ref() {
                let text =
                    serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
                job_box = job_box.child(section("CLOUD RESULT", text, palette));
            }

            panel = panel.child(job_box);
        }

        let current_class = if item.data_class.value.is_empty() {
            "c1"
        } else {
            item.data_class.value.as_str()
        };
        let current_label = if item.data_class.label.is_empty() {
            match current_class {
                "c0" => "Public",
                "c1" => "Mine",
                "c2" => "Others",
                _ => "Secret",
            }
        } else {
            item.data_class.label.as_str()
        };
        let source = self.content_source.unwrap_or("feed");
        let classes: &[(&'static str, &'static str)] = if source == "mail" {
            &[
                ("c1", "Mine · C1"),
                ("c2", "Others · C2"),
                ("c3", "Secret · C3"),
            ]
        } else {
            &[
                ("c0", "Public · C0"),
                ("c1", "Mine · C1"),
                ("c2", "Others · C2"),
                ("c3", "Secret · C3"),
            ]
        };
        let class_buttons = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(classes.iter().map(|&(code, label)| {
                let selected = current_class == code;
                Button::new(SharedString::from(format!("feed-data-class-{code}")))
                    .xsmall()
                    .label(label)
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.ghost())
                    .disabled(self.data_class_busy)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.select_data_class(code, window, cx);
                    }))
            }));
        let mut class_box = div()
            .mt_2()
            .p_3()
            .rounded(px(6.))
            .border_1()
            .border_color(rgba(palette.border))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!(
                                "DATA CLASS · {} ({})",
                                current_label,
                                current_class.to_uppercase()
                            )),
                    )
                    .child(muted(&item.data_class.method, palette)),
            )
            .when(!item.data_class.rationale.trim().is_empty(), |el| {
                el.child(muted(&item.data_class.rationale, palette))
            })
            .child(class_buttons);

        if let Some(form) = self.data_class_form.as_ref() {
            class_box = class_box.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(muted(
                        &format!(
                            "Lowering from {} to {} requires a rationale.",
                            current_class.to_uppercase(),
                            form.target.to_uppercase()
                        ),
                        palette,
                    ))
                    .child(Input::new(&form.rationale).small())
                    .when_some(form.error.as_deref(), |el, err| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.red))
                                .child(err.to_owned()),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("feed-data-class-cancel")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .disabled(self.data_class_busy)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.data_class_form = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("feed-data-class-confirm")
                                    .small()
                                    .primary()
                                    .label(if self.data_class_busy {
                                        "Saving…"
                                    } else {
                                        "Confirm lower class"
                                    })
                                    .disabled(self.data_class_busy)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.confirm_data_class_lowering(cx);
                                    })),
                            ),
                    ),
            );
        }

        if let Some(error) = self.data_class_error.as_ref() {
            class_box = class_box.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(format!("Data class update failed: {error}")),
            );
        }

        panel = panel.child(class_box);
        panel.into_any_element()
    }

    pub(crate) fn calendar_edit_panel(
        &self,
        palette: Palette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let form = self.calendar_edit.as_ref()?;
        let mut fields = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(Input::new(&form.title).small())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&form.start_date).small()))
                    .child(div().flex_1().child(Input::new(&form.end_date).small())),
            );
        if !form.all_day {
            fields = fields.child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&form.start_time).small()))
                    .child(div().flex_1().child(Input::new(&form.end_time).small())),
            );
        }
        fields = fields
            .child(Input::new(&form.location).small())
            .child(Input::new(&form.notes).small());

        if let Some(error) = &form.error {
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }

        Some(
            div()
                .p_3()
                .rounded(px(6.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.raised))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Edit Calendar Entry"),
                )
                .child(fields)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("calendar-edit-cancel")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.calendar_edit = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("calendar-edit-save")
                                .small()
                                .primary()
                                .label(if form.saving {
                                    "Saving…"
                                } else {
                                    "Save changes"
                                })
                                .disabled(form.saving)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.save_calendar_edit(cx)),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// The entries `filter` and `stream` show, in the order the list draws them: newest day first,
/// best score first within a day.
pub(crate) fn in_view<'a>(
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

pub(crate) fn title(title: Option<&str>, url: &str) -> String {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(url)
        .to_owned()
}

pub(crate) fn meta(entry: &FeedEntry) -> String {
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

pub(crate) fn date_part(value: &str) -> String {
    value.get(..10).unwrap_or_default().to_owned()
}

pub(crate) fn time_part(value: &str) -> String {
    value.get(11..16).unwrap_or("09:00").to_owned()
}

pub(crate) fn muted(text: &str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.to_owned())
}

pub(crate) fn section(label: &str, body: String, palette: Palette) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(render_markdown(&body, palette, Density::Desktop))
}
