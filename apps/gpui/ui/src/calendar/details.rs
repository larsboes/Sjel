//! Selected event provenance, navigation and quick rescheduling beside Ely's agenda.
use super::types::View;
use super::{
    adapter,
    page::CalendarPage,
    types::{muted, CalendarIntent},
};
use crate::theme::tint;
use crate::Palette;
use chrono::Local;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::calendar::{self, kind_color, kind_label, time_label};

impl CalendarPage {
    pub(super) fn day_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.day_entries();
        let is_today = self.day == Local::now().date_naive();
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(self.day.format("%A, %b %-d").to_string()),
                    )
                    .when(is_today, |el| {
                        el.child(
                            div()
                                .px_1p5()
                                .py(px(1.))
                                .rounded_full()
                                .bg(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0xffffff))
                                .child("Today"),
                        )
                    }),
            )
            .child(
                Button::new("day-panel-add")
                    .xsmall()
                    .ghost()
                    .label("+ Add")
                    .on_click(cx.listener(|page, _, window, cx| page.open_form(window, cx))),
            );

        let mut panel = div().flex().flex_col().gap_3().child(header);
        if self.selected_entry.is_some() {
            panel = panel.child(
                Button::new("calendar-edit-selected")
                    .small()
                    .ghost()
                    .label("Edit entry · Enter")
                    .disabled(self.changing_time)
                    .on_click(cx.listener(|page, _, window, cx| page.edit_selected(window, cx))),
            );
        }
        if self.view == View::Day && self.entries.error.is_none() && !self.entries.loading {
            let all = self.entries.value.as_deref().unwrap_or_default();
            let now = Local::now();
            let next = calendar::timeline::blocks(all, self.day)
                .into_iter()
                .find(|block| {
                    !is_today
                        || block.end as u32
                            > chrono::Timelike::hour(&now) * 60 + chrono::Timelike::minute(&now)
                });
            if let Some(block) = next {
                panel = panel.child(div().text_sm().child(format!(
                    "{} · {}",
                    if is_today {
                        "Up next / now"
                    } else {
                        "First event"
                    },
                    all[block.entry_index].title
                )));
            }
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Free time shows gaps from 08:00–20:00; planned events reserve time."),
            );
        }

        let (events, _) = self.events(&entries);
        let (select, open) = (cx.entity().downgrade(), cx.entity().downgrade());
        panel = panel.child(
            crate::ely::calendar::AgendaView::new(
                "calendar-agenda",
                adapter::date(self.day),
                1,
                events,
            )
            .zone(self.zone.clone())
            .selected(self.selected_entry.clone().map(Into::into))
            .on_select(move |id, _, cx| {
                let _ = select.update(cx, |page, cx| page.activate_event(id, cx));
            })
            .on_event(move |id, window, cx| {
                let _ = open.update(cx, |page, cx| {
                    page.activate_event(id, cx);
                    page.edit_selected(window, cx);
                });
            }),
        );
        if self.entries.loading && self.entries.value.is_none() {
            panel = panel.child(muted("Loading…", palette));
        } else if entries.is_empty() {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .py_6()
                    .px_3()
                    .gap_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.raised))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.muted))
                            .child("No events scheduled"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Press 'n' or click + Add above"),
                    ),
            );
        } else {
            for (i, entry) in entries.into_iter().enumerate() {
                let color = kind_color(&entry.kind);
                let chosen = self.selected_entry.as_deref() == Some(entry.id.as_str());
                let id = entry.id.clone();
                let confirming =
                    chosen && self.confirm_delete.as_deref() == Some(entry.id.as_str());
                let time_str = time_label(&entry, self.day);
                let kind_str = kind_label(&entry.kind).to_owned();

                let (origin_item_id, importance_rationale, origin_cap, origin_source, plan_id) =
                    if chosen {
                        self.selected_detail
                            .as_ref()
                            .filter(|(detail_id, _)| {
                                Some(detail_id.as_str()) == self.selected_entry.as_deref()
                            })
                            .and_then(|(_, detail)| detail.payload.as_ref())
                            .map(|payload| {
                                let origin = payload.get("origin").and_then(|o| o.as_object());
                                let item_id = origin
                                    .and_then(|o| o.get("item_id"))
                                    .and_then(|i| i.as_str())
                                    .map(str::to_string);
                                let cap = origin
                                    .and_then(|o| o.get("capability"))
                                    .and_then(|c| c.as_str())
                                    .map(str::to_string);
                                let src = origin
                                    .and_then(|o| o.get("source"))
                                    .and_then(|s| s.as_str())
                                    .map(str::to_string);
                                let rationale = payload
                                    .get("importance_rationale")
                                    .and_then(|r| r.as_str())
                                    .map(str::to_string);
                                let plan_id = payload
                                    .get("plan_id")
                                    .and_then(|id| id.as_str())
                                    .map(str::to_string);
                                (item_id, rationale, cap, src, plan_id)
                            })
                            .unwrap_or((None, None, None, None, None))
                    } else {
                        (None, None, None, None, None)
                    };

                let is_comms = entry.source == "comms"
                    || entry.source == "mail"
                    || entry.source == "feed"
                    || origin_cap.as_deref() == Some("comms")
                    || origin_source.as_deref() == Some("mail");
                let is_trips = entry.source == "trips" || origin_cap.as_deref() == Some("trips");

                panel = panel.child(
                    div()
                        .id(("calendar-entry", i))
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .py_2()
                        .px_2p5()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(if chosen {
                            rgb(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .bg(if chosen {
                            rgba(palette.raised)
                        } else {
                            rgba(tint(color, 0x0c))
                        })
                        .hover(|el| {
                            if !chosen {
                                el.bg(rgba(tint(color, 0x1c)))
                            } else {
                                el
                            }
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.select_entry(id.clone(), cx);
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .w(px(6.))
                                                .h(px(6.))
                                                .flex_none()
                                                .rounded_full()
                                                .bg(rgb(color)),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(color))
                                                .child(kind_str),
                                        )
                                        .when(
                                            entry.commitment == "tentative"
                                                || entry.commitment == "possible",
                                            |el| {
                                                el.child(
                                                    div()
                                                        .px_1p5()
                                                        .py(px(0.5))
                                                        .rounded(px(3.))
                                                        .border_1()
                                                        .border_color(rgba(palette.border))
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .child(entry.commitment.clone()),
                                                )
                                            },
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .when(is_comms, |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("✉ Mail"),
                                            )
                                        })
                                        .when(is_trips, |el| {
                                            el.child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("✈ Travel"),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(palette.muted))
                                                .child(time_str),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(entry.title.clone()),
                        )
                        .when_some(
                            entry.location.clone().filter(|l| chosen && !l.is_empty()),
                            |el, l| {
                                el.child(div().text_xs().text_color(rgb(palette.muted)).child(l))
                            },
                        )
                        .when_some(
                            entry.notes.clone().filter(|n| chosen && !n.is_empty()),
                            |el, n| {
                                el.child(
                                    div()
                                        .p_2()
                                        .rounded(px(4.))
                                        .bg(rgba(palette.surface))
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(n),
                                )
                            },
                        )
                        .when(chosen && is_comms, |el| {
                            let origin_id = origin_item_id.clone();
                            el.child(
                                div()
                                    .mt_1()
                                    .p_2p5()
                                    .rounded(px(6.))
                                    .bg(rgba(palette.surface))
                                    .border_1()
                                    .border_color(rgba(palette.border))
                                    .flex()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.accent))
                                                    .child("Source: Mail / Comms"),
                                            )
                                            .child(
                                                Button::new("calendar-open-source-mail")
                                                    .xsmall()
                                                    .primary()
                                                    .label("Open in Mail ↗")
                                                    .on_click(cx.listener(move |_, _, _, cx| {
                                                        cx.emit(CalendarIntent::OpenMail {
                                                            item_id: origin_id.clone(),
                                                        });
                                                    })),
                                            ),
                                    )
                                    .when_some(importance_rationale, |card, reason| {
                                        card.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child(reason),
                                        )
                                    }),
                            )
                        })
                        .when(chosen && is_trips, |el| {
                            let plan_id = plan_id.clone();
                            el.child(
                                div()
                                    .mt_1()
                                    .p_2p5()
                                    .rounded(px(6.))
                                    .bg(rgba(palette.surface))
                                    .border_1()
                                    .border_color(rgba(palette.border))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.accent))
                                            .child("Source: Travel / Trips"),
                                    )
                                    .child(
                                        Button::new("calendar-open-source-travel")
                                            .xsmall()
                                            .primary()
                                            .label("View in Travel ↗")
                                            .disabled(plan_id.is_none())
                                            .on_click(cx.listener(move |_, _, _, cx| {
                                                if let Some(plan_id) = &plan_id {
                                                    cx.emit(CalendarIntent::OpenTrip {
                                                        plan_id: plan_id.clone(),
                                                    });
                                                }
                                            })),
                                    ),
                            )
                        })
                        .when(
                            chosen
                                && !entry.source.is_empty()
                                && entry.source != "manual"
                                && !is_comms
                                && !is_trips,
                            |el| {
                                el.child(
                                    div()
                                        .mt_1()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!("From {}", entry.source)),
                                )
                            },
                        )
                        .when(chosen, |card| {
                            let entry_id = entry.id.clone();
                            let all_day = entry.all_day;
                            let mut bar = div()
                                .mt_2()
                                .pt_2()
                                .border_t_1()
                                .border_color(rgba(palette.border))
                                .flex()
                                .flex_col()
                                .gap_1p5();

                            let mut day_row =
                                div().flex().flex_wrap().gap_1p5().items_center().child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child("Reschedule:"),
                                );
                            for (label, days) in [("Tomorrow", 1), ("+7d", 7), ("−1d", -1)] {
                                let eid = entry_id.clone();
                                day_row = day_row.child(
                                    Button::new(SharedString::from(format!(
                                        "calendar-quick-day-{days}-{eid}"
                                    )))
                                    .xsmall()
                                    .ghost()
                                    .label(label)
                                    .disabled(self.changing_time)
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.quick_shift_days(&eid, days, cx);
                                    })),
                                );
                            }
                            bar = bar.child(day_row);

                            if !all_day {
                                let mut time_row =
                                    div().flex().flex_wrap().gap_1p5().items_center().child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.muted))
                                            .child("Time:"),
                                    );
                                for (label, delta, resize) in [
                                    ("−15m", -15, false),
                                    ("+15m", 15, false),
                                    ("−1h", -60, false),
                                    ("+1h", 60, false),
                                    ("−15m len", -15, true),
                                    ("+15m len", 15, true),
                                ] {
                                    let eid = entry_id.clone();
                                    time_row =
                                        time_row.child(
                                            Button::new(SharedString::from(format!(
                                                "calendar-quick-time-{label}-{eid}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .label(label)
                                            .disabled(self.changing_time)
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.quick_shift_minutes(&eid, delta, resize, cx);
                                            })),
                                        );
                                }
                                bar = bar.child(time_row);
                            }

                            if self.undo_time.is_some() {
                                bar =
                                    bar.child(
                                        div().flex().items_center().justify_end().child(
                                            Button::new(SharedString::from(format!(
                                                "calendar-quick-undo-{entry_id}"
                                            )))
                                            .xsmall()
                                            .ghost()
                                            .label("Undo change ⎌")
                                            .disabled(self.changing_time)
                                            .on_click(cx.listener(|page, _, _, cx| {
                                                page.undo_time(cx);
                                            })),
                                        ),
                                    );
                            }

                            card.child(bar)
                        })
                        .when(confirming, |el| {
                            let from = match entry.source.as_str() {
                                "" | "manual" => String::new(),
                                source => format!(" (from {source})"),
                            };
                            el.child(
                                div()
                                    .p_2()
                                    .rounded(px(4.))
                                    .bg(rgba(0xef444426))
                                    .border_1()
                                    .border_color(rgb(palette.red))
                                    .text_xs()
                                    .text_color(rgb(palette.red))
                                    .child(format!(
                                        "Delete this entry{from}? Enter confirms · Esc cancels"
                                    )),
                            )
                        }),
                );
            }
        }
        panel.into_any_element()
    }
}
